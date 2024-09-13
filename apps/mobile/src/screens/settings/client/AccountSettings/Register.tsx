import { zodResolver } from '@hookform/resolvers/zod';
import { useNavigation } from '@react-navigation/native';
import { useState } from 'react';
import { Controller, useForm } from 'react-hook-form';
import { Text, View } from 'react-native';
import { signUp } from 'supertokens-web-js/recipe/emailpassword';
import { z } from 'zod';
import { telemetryState } from '@sd/client';
import { Button } from '~/components/primitive/Button';
import { Input } from '~/components/primitive/Input';
import { toast } from '~/components/primitive/Toast';
import { tw } from '~/lib/tailwind';
import { SettingsStackScreenProps } from '~/navigation/tabs/SettingsStack';

import ShowPassword from './ShowPassword';

const RegisterSchema = z
	.object({
		email: z.string().email(),
		password: z.string().min(6),
		confirmPassword: z.string().min(6)
	})
	.refine((data) => data.password === data.confirmPassword, {
		message: 'Passwords do not match',
		path: ['confirmPassword']
	});
type RegisterType = z.infer<typeof RegisterSchema>;

async function signUpClicked(
	email: string,
	password: string,
	navigator: SettingsStackScreenProps<'AccountProfile'>['navigation']
) {
	try {
		const req = await fetch('http://localhost:9000/api/auth/signup', {
			method: 'POST',
			headers: {
				'Content-Type': 'application/json; charset=utf-8'
			},
			body: JSON.stringify({
				formFields: [
					{
						id: 'email',
						value: email
					},
					{
						id: 'password',
						value: password
					}
				]
			})
		});

		const response: {
			status: string;
			reason?: string;
			user?: {
				id: string;
				email: string;
				timeJoined: number;
				tenantIds: string[];
			};
		} = await req.json();

		if (response.status === 'FIELD_ERROR') {
			// one of the input formFields failed validaiton
			console.error('Field error: ', response.reason);
		} else if (response.status === 'SIGN_UP_NOT_ALLOWED') {
			// the reason string is a user friendly message
			// about what went wrong. It can also contain a support code which users
			// can tell you so you know why their sign up was not allowed.
			toast.error(response.reason!);
		} else {
			// sign up successful. The session tokens are automatically handled by
			// the frontend SDK.
			toast.success('Sign up successful');
			navigator.navigate('AccountProfile');
		}
	} catch (err: any) {
		if (err.isSuperTokensGeneralError === true) {
			// this may be a custom error message sent from the API by you.
			toast.error(err.message);
		} else {
			toast.error('Oops! Something went wrong.');
		}
	}
}

const Register = () => {
	const [showPassword, setShowPassword] = useState(false);
	// useZodForm seems to be out-dated or needs
	//fixing as it does not support the schema using zod.refine
	const form = useForm<RegisterType>({
		resolver: zodResolver(RegisterSchema),
		defaultValues: {
			email: '',
			password: '',
			confirmPassword: ''
		}
	});

	const navigator = useNavigation<SettingsStackScreenProps<'AccountProfile'>['navigation']>();
	return (
		<View style={tw`flex flex-col gap-1.5`}>
			<Controller
				control={form.control}
				name="email"
				render={({ field }) => (
					<Input {...field} placeholder="Email" onChangeText={field.onChange} />
				)}
			/>
			{form.formState.errors.email && (
				<Text style={tw`text-xs text-red-500`}>{form.formState.errors.email.message}</Text>
			)}
			<Controller
				control={form.control}
				name="password"
				render={({ field }) => (
					<View style={tw`relative flex items-center justify-center`}>
						<Input
							{...field}
							placeholder="Password"
							style={tw`w-full`}
							onChangeText={field.onChange}
							secureTextEntry={!showPassword}
						/>
					</View>
				)}
			/>
			{form.formState.errors.password && (
				<Text style={tw`text-xs text-red-500`}>
					{form.formState.errors.password.message}
				</Text>
			)}
			<Controller
				control={form.control}
				name="confirmPassword"
				render={({ field }) => (
					<View style={tw`relative flex items-center justify-center`}>
						<Input
							{...field}
							placeholder="Confirm Password"
							style={tw`w-full`}
							onChangeText={field.onChange}
							secureTextEntry={!showPassword}
						/>
						<ShowPassword
							showPassword={showPassword}
							setShowPassword={setShowPassword}
							plural={true}
						/>
					</View>
				)}
			/>
			{form.formState.errors.confirmPassword && (
				<Text style={tw`text-xs text-red-500`}>
					{form.formState.errors.confirmPassword.message}
				</Text>
			)}
			<Button
				style={tw`mx-auto mt-2 w-full`}
				variant="accent"
				onPress={form.handleSubmit(async (data) => {
					console.log(data);
					await signUpClicked(data.email, data.password, navigator);
				})}
				disabled={form.formState.isSubmitting}
			>
				<Text>Submit</Text>
			</Button>
		</View>
	);
};

export default Register;